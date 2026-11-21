//! Data sources: rows of named values converted to the classes the template declares.

use crate::error::{Error, Result};
use crate::model::Report;
use crate::value::{JavaType, Value};
use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use std::collections::HashMap;

/// One row of the data source.
pub type Record = HashMap<String, Value>;

/// Key under which a record keeps its source JSON object (for `subDataSource`).
pub const ROW_KEY: &str = "\u{0}row";

/// Converts a JSON value to `ty`.
pub fn json_to_value(v: &serde_json::Value, ty: JavaType) -> Result<Value> {
    use serde_json::Value as J;
    if v.is_null() {
        return Ok(Value::Null);
    }
    let bad = || Error::Data(format!("cannot convert {v} to {ty:?}"));
    let num_str = |v: &J| -> Option<String> {
        match v {
            J::Number(n) => Some(n.to_string()),
            J::String(s) => Some(s.trim().to_string()),
            _ => None,
        }
    };
    Ok(match ty {
        JavaType::String => Value::str(match v {
            J::String(s) => s.clone(),
            other => other.to_string(),
        }),
        JavaType::Integer | JavaType::Short | JavaType::Byte => {
            Value::Int(num_str(v).and_then(|s| s.parse::<f64>().ok()).map(|f| f as i32).ok_or_else(bad)?)
        }
        JavaType::Long | JavaType::BigInteger => {
            Value::Long(num_str(v).and_then(|s| s.parse::<f64>().ok()).map(|f| f as i64).ok_or_else(bad)?)
        }
        JavaType::Double | JavaType::Number => Value::Double(num_str(v).and_then(|s| s.parse().ok()).ok_or_else(bad)?),
        JavaType::Float => Value::Float(num_str(v).and_then(|s| s.parse().ok()).ok_or_else(bad)?),
        JavaType::BigDecimal => Value::Decimal(
            num_str(v)
                .and_then(|s| Decimal::from_str_exact(&s).ok().or_else(|| Decimal::from_scientific(&s).ok()))
                .ok_or_else(bad)?,
        ),
        JavaType::Boolean => Value::Bool(match v {
            J::Bool(b) => *b,
            J::String(s) => s.eq_ignore_ascii_case("true"),
            J::Number(n) => n.as_f64() != Some(0.0),
            _ => return Err(bad()),
        }),
        JavaType::Date | JavaType::Timestamp | JavaType::SqlDate | JavaType::Time => match v {
            J::String(s) => Value::Date(parse_datetime(s).ok_or_else(bad)?),
            J::Number(n) => {
                Value::Date(chrono::DateTime::from_timestamp_millis(n.as_i64().ok_or_else(bad)?).ok_or_else(bad)?.naive_utc())
            }
            _ => return Err(bad()),
        },
        JavaType::Bytes => match v {
            J::String(s) => {
                let b64 = match s.strip_prefix("data:") {
                    Some(rest) => rest.split_once(";base64,").map(|(_, b)| b).unwrap_or(rest),
                    None => s.as_str(),
                };
                Value::Bytes(crate::image::decode_base64(b64)?.into())
            }
            _ => return Err(bad()),
        },
        JavaType::List => Value::Json(std::sync::Arc::new(v.clone())),
        JavaType::Object => match v {
            J::String(s) => Value::str(s.clone()),
            J::Bool(b) => Value::Bool(*b),
            J::Number(n) if n.is_i64() => Value::Long(n.as_i64().unwrap_or(0)),
            J::Number(n) => Value::Double(n.as_f64().unwrap_or(0.0)),
            other => Value::Json(std::sync::Arc::new(other.clone())),
        },
    })
}

/// Parses ISO-8601 dates and date-times (`2026-01-15`, `2026-01-15T10:30:00`, with optional
/// fractional seconds and `Z`).
pub fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    // With an explicit offset: keep the local date and time it was written with.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s.trim()) {
        if !s.trim().ends_with('Z') {
            return Some(dt.naive_local());
        }
    }
    let s = s.trim().trim_end_matches('Z');
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt);
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// Builds records from a JSON array of objects, typed according to the report's fields.
/// Keys may be dotted paths (`customer.name`) into nested objects.
pub fn records_from_json(report: &Report, json: &serde_json::Value) -> Result<Vec<Record>> {
    let rows = match json {
        serde_json::Value::Array(rows) => rows,
        _ => return Err(Error::Data("expected a JSON array of objects".into())),
    };
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let obj = row.as_object().ok_or_else(|| Error::Data(format!("row {i} is not an object")))?;
            let mut rec = Record::new();
            if report.uses_data_source {
                rec.insert(ROW_KEY.to_string(), Value::Json(std::sync::Arc::new(row.clone())));
            }
            for field in &report.fields {
                let v = obj.get(&field.name).or_else(|| lookup_path(row, &field.name));
                let value = match v {
                    Some(v) => {
                        json_to_value(v, field.class).map_err(|e| Error::Data(format!("row {i}, field `{}`: {e}", field.name)))?
                    }
                    None => Value::Null,
                };
                rec.insert(field.name.clone(), value);
            }
            Ok(rec)
        })
        .collect()
}

fn lookup_path<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.').try_fold(v, |cur, key| cur.get(key))
}
