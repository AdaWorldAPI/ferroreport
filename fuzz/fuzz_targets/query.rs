//! Any text as a report's SQL query: `$P{}`, `$P!{}` and `$X{}` are replaced without panics.
#![no_main]
use ferroreport::value::Value;
use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let mut params = HashMap::new();
    params.insert("A".to_string(), Value::Int(1));
    params.insert("L".to_string(), Value::Json(std::sync::Arc::new(serde_json::json!(["x", null, 2]))));
    params.insert("N".to_string(), Value::Null);
    let _ = ferroreport::sql::prepare(text, &params, &|n| format!("${n}"));
});
