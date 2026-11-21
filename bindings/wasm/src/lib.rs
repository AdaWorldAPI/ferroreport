//! WebAssembly bindings: `render(template, data, params, options)` returning the PDF (or XLSX or
//! CSV) bytes.

use js_sys::{Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn to_json(v: &JsValue, default: serde_json::Value) -> Result<serde_json::Value, JsError> {
    if v.is_undefined() || v.is_null() {
        return Ok(default);
    }
    if let Some(s) = v.as_string() {
        return serde_json::from_str(&s).map_err(js_err);
    }
    serde_wasm_bindgen::from_value(v.clone()).map_err(|e| JsError::new(&e.to_string()))
}

fn get(obj: &JsValue, key: &str) -> JsValue {
    if obj.is_object() {
        Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
    } else {
        JsValue::UNDEFINED
    }
}

/// Renders a JRXML template to PDF (or to Excel or CSV with `options.format`).
///
/// - `template`: the JRXML source.
/// - `data`: rows (an array of objects) or JSON text.
/// - `params`: report parameters (an object) or JSON text.
/// - `options`: `{ locale?: string, now?: string | Date, resources?: { [path]: Uint8Array | string },
///   format?: "pdf" | "xlsx" | "csv" | "html", maxPages?: number, csvSafe?: boolean, xlsxNumbers?: boolean }`.
///   `maxPages` stops a report that grows past that many pages; `csvSafe` puts an apostrophe
///   before CSV values a spreadsheet would run as a formula; `xlsxNumbers` writes numbers as
///   numbers in Excel instead of formatted text.
#[wasm_bindgen]
pub fn render(template: &str, data: JsValue, params: JsValue, options: JsValue) -> Result<Vec<u8>, JsError> {
    let rows = to_json(&data, serde_json::Value::Array(Vec::new()))?;
    let params = to_json(&params, serde_json::Value::Object(Default::default()))?;
    let locale = get(&options, "locale").as_string().unwrap_or_else(|| "en_US".into());
    let limits =
        ferroreport::Limits { max_pages: get(&options, "maxPages").as_f64().map(|n| n.max(0.0) as usize), timeout: None };
    let mut r = ferroreport::Renderer::new()
        .locale(&locale)
        .limits(limits)
        .xlsx_options(ferroreport::xlsx::XlsxOptions { detect_cell_type: get(&options, "xlsxNumbers").is_truthy() })
        .csv_options(ferroreport::csv::CsvOptions { guard_formulas: get(&options, "csvSafe").is_truthy(), ..Default::default() });

    // `new Date()` in expressions: the given value, or the current time from JavaScript.
    let now = get(&options, "now");
    let now = if let Some(s) = now.as_string() {
        s
    } else if now.is_instance_of::<js_sys::Date>() {
        String::from(js_sys::Date::from(now).to_iso_string())
    } else {
        String::from(js_sys::Date::new_0().to_iso_string())
    };
    let now = ferroreport::data::parse_datetime(now.trim_end_matches('Z'))
        .ok_or_else(|| JsError::new(&format!("options.now: cannot parse `{now}`")))?;
    r = r.now(now);

    let resources = get(&options, "resources");
    if resources.is_object() {
        for entry in Object::entries(&Object::from(resources)).iter() {
            let pair = js_sys::Array::from(&entry);
            let name = pair.get(0).as_string().unwrap_or_default();
            let value = pair.get(1);
            let bytes = match value.as_string() {
                Some(s) => s.into_bytes(),
                None => Uint8Array::new(&value).to_vec(),
            };
            r.add_resource(&name, bytes);
        }
    }
    let format = match get(&options, "format").as_string() {
        Some(f) => ferroreport::Format::parse(&f)
            .ok_or_else(|| JsError::new(&format!("options.format: unknown format `{f}` (pdf, xlsx, csv or html)")))?,
        None => ferroreport::Format::Pdf,
    };
    r.render_json_as(template, &rows, &params, format).map_err(js_err)
}

/// Parses a template: `{ name, parameters, fields, warnings }`, where `warnings` lists what
/// this engine does not support in it.
#[wasm_bindgen]
pub fn inspect(template: &str) -> Result<JsValue, JsError> {
    let report = ferroreport::parse(template).map_err(js_err)?;
    let out = serde_json::json!({
        "name": report.name,
        "parameters": report.parameters.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        "fields": report.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        "warnings": report.warnings,
    });
    let ser = serde_wasm_bindgen::Serializer::json_compatible();
    serde::Serialize::serialize(&out, &ser).map_err(|e| JsError::new(&e.to_string()))
}

/// Version of the engine.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
