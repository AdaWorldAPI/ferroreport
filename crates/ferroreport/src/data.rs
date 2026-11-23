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

/// Builds one record from a JSON object, typed according to `fields`.
fn record_from_json(fields: &[crate::model::Field], keep_row: bool, i: usize, row: &serde_json::Value) -> Result<Record> {
    let obj = row.as_object().ok_or_else(|| Error::Data(format!("row {i} is not an object")))?;
    let mut rec = Record::with_capacity(fields.len() + keep_row as usize);
    if keep_row {
        rec.insert(ROW_KEY.to_string(), Value::Json(std::sync::Arc::new(row.clone())));
    }
    for field in fields {
        let v = obj.get(&field.name).or_else(|| lookup_path(row, &field.name));
        let value = match v {
            Some(v) => json_to_value(v, field.class).map_err(|e| Error::Data(format!("row {i}, field `{}`: {e}", field.name)))?,
            None => Value::Null,
        };
        rec.insert(field.name.clone(), value);
    }
    Ok(rec)
}

/// Builds records from a JSON array of objects, typed according to the report's fields.
/// Keys may be dotted paths (`customer.name`) into nested objects.
pub fn records_from_json(report: &Report, json: &serde_json::Value) -> Result<Vec<Record>> {
    let rows = match json {
        serde_json::Value::Array(rows) => rows,
        _ => return Err(Error::Data("expected a JSON array of objects".into())),
    };
    rows.iter().enumerate().map(|(i, row)| record_from_json(&report.fields, report.uses_data_source, i, row)).collect()
}

/// Rows read one at a time from JSON text, so a large data file is never in memory whole.
/// The text is an array of objects (`[{...}, {...}]`) or one object after another
/// (JSON Lines).
pub struct JsonRows<R: std::io::Read> {
    reader: R,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    shape: Shape,
    /// In an array: a value was read, so a comma or the closing bracket comes next.
    after_value: bool,
    index: usize,
    fields: Vec<crate::model::Field>,
    keep_row: bool,
    done: bool,
    /// No text at all means no rows, instead of an error.
    empty_ok: bool,
    /// Largest a single row may be, in bytes of JSON.
    max_row: usize,
}

/// Largest a single row may be by default (256 MiB of JSON): enough for rows that carry
/// pictures, and a stop for data that is one endless value.
pub const MAX_ROW_BYTES: usize = 256 << 20;

#[derive(PartialEq)]
enum Shape {
    Unknown,
    Array,
    Lines,
}

impl<R: std::io::Read> JsonRows<R> {
    pub fn new(report: &Report, reader: R) -> JsonRows<R> {
        JsonRows {
            reader,
            buf: Vec::new(),
            pos: 0,
            eof: false,
            shape: Shape::Unknown,
            after_value: false,
            index: 0,
            fields: report.fields.clone(),
            keep_row: report.uses_data_source,
            done: false,
            empty_ok: false,
            max_row: MAX_ROW_BYTES,
        }
    }

    /// Stops with an error at a row larger than `bytes` of JSON, before reading more of it.
    pub fn max_row_bytes(mut self, bytes: usize) -> JsonRows<R> {
        self.max_row = bytes.max(1);
        self
    }

    /// Takes text with nothing in it as no rows. For rows that follow something else (the
    /// lines after the first one of a request): a report without data has none.
    pub fn allow_empty(mut self) -> JsonRows<R> {
        self.empty_ok = true;
        self
    }

    /// Drops what was consumed and reads more text.
    fn more(&mut self) -> Result<()> {
        self.buf.drain(..self.pos);
        self.pos = 0;
        // What is buffered now is one row not yet complete.
        if self.buf.len() > self.max_row {
            return Err(Error::Limit(format!("row {} is larger than {} MB of JSON", self.index + 1, self.max_row >> 20)));
        }
        // Grow by what is buffered (or 256 KiB), but never past one byte over the limit, so a
        // row too large is found before more memory than the limit is taken for it.
        let old = self.buf.len();
        let grow = old.max(256 * 1024).min(self.max_row + 1 - old);
        self.buf.reserve_exact(grow);
        self.buf.resize(old + grow, 0);
        let n = loop {
            match self.reader.read(&mut self.buf[old..]) {
                Ok(n) => break n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(Error::Data(format!("cannot read the data: {e}"))),
            }
        };
        self.buf.truncate(old + n);
        self.eof = n == 0;
        Ok(())
    }

    fn next_value(&mut self) -> Result<Option<serde_json::Value>> {
        loop {
            while self.pos < self.buf.len() && self.buf[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos == self.buf.len() {
                if !self.eof {
                    self.more()?;
                    continue;
                }
                return match self.shape {
                    Shape::Array => Err(Error::Data("the JSON array of rows is not closed".into())),
                    Shape::Lines => Ok(None),
                    Shape::Unknown if self.empty_ok => Ok(None),
                    Shape::Unknown => Err(Error::Data("expected a JSON array of objects".into())),
                };
            }
            let b = self.buf[self.pos];
            match self.shape {
                Shape::Unknown => {
                    match b {
                        b'[' => {
                            self.shape = Shape::Array;
                            self.pos += 1;
                        }
                        b'{' => self.shape = Shape::Lines,
                        // A UTF-8 byte order mark.
                        0xEF if self.buf[self.pos..].starts_with(&[0xEF, 0xBB, 0xBF]) => self.pos += 3,
                        0xEF if self.buf.len() - self.pos < 3 && !self.eof => self.more()?,
                        _ => return Err(Error::Data("expected a JSON array of objects".into())),
                    }
                    continue;
                }
                Shape::Array if self.after_value => {
                    self.pos += 1;
                    match b {
                        b',' => self.after_value = false,
                        b']' => return Ok(None),
                        _ => return Err(Error::Data(format!("after row {}: expected `,` or `]`", self.index.saturating_sub(1)))),
                    }
                    continue;
                }
                Shape::Array if b == b']' => return Ok(None),
                _ => {}
            }
            let mut values = serde_json::Deserializer::from_slice(&self.buf[self.pos..]).into_iter::<serde_json::Value>();
            match values.next() {
                Some(Ok(v)) => {
                    let used = values.byte_offset();
                    // Anything but an object or array may continue in text not read yet.
                    if self.pos + used == self.buf.len() && !self.eof && !(v.is_object() || v.is_array()) {
                        self.more()?;
                        continue;
                    }
                    self.pos += used;
                    self.after_value = true;
                    return Ok(Some(v));
                }
                Some(Err(e)) if e.is_eof() && !self.eof => self.more()?,
                Some(Err(e)) => return Err(Error::Data(format!("row {}: {e}", self.index))),
                None => self.more()?,
            }
        }
    }
}

impl<R: std::io::Read> Iterator for JsonRows<R> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Result<Record>> {
        if self.done {
            return None;
        }
        let row = match self.next_value() {
            Ok(Some(v)) => record_from_json(&self.fields, self.keep_row, self.index, &v),
            Ok(None) => {
                self.done = true;
                return None;
            }
            Err(e) => Err(e),
        };
        self.index += 1;
        self.done = row.is_err();
        Some(row)
    }
}

fn lookup_path<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.').try_fold(v, |cur, key| cur.get(key))
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    /// Hands out the text a few bytes at a time, like a slow socket.
    struct Drip<'a>(&'a [u8], usize);
    impl std::io::Read for Drip<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.1.min(self.0.len()).min(out.len());
            out[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    fn report() -> Report {
        crate::parse(
            r#"<jasperReport name="t" pageWidth="200" pageHeight="200" columnWidth="200" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
            <field name="id" class="java.lang.Integer"/><field name="name" class="java.lang.String"/></jasperReport>"#,
        )
        .unwrap()
    }

    fn show(records: Vec<Record>) -> Vec<String> {
        records.iter().map(|r| format!("{:?} {:?}", r["id"], r["name"])).collect()
    }

    fn read(text: &str, step: usize) -> Result<Vec<String>> {
        JsonRows::new(&report(), Drip(text.as_bytes(), step)).collect::<Result<Vec<Record>>>().map(show)
    }

    #[test]
    fn stops_at_a_row_larger_than_the_limit() {
        let big = format!("[{{\"id\": 1, \"name\": \"{}\"}}]", "x".repeat(3 << 20));
        let rows = JsonRows::new(&report(), Drip(big.as_bytes(), 64 << 10)).max_row_bytes(1 << 20);
        let e = rows.collect::<Result<Vec<Record>>>().unwrap_err();
        assert!(e.to_string().contains("row 1 is larger than 1 MB"), "{e}");
        // The buffer never holds more than the limit and one byte.
        let mut rows = JsonRows::new(&report(), Drip(big.as_bytes(), 64 << 10)).max_row_bytes(1 << 20);
        assert!(rows.next().unwrap().is_err());
        assert!(rows.buf.capacity() <= (1 << 20) + 1 + (256 << 10), "{}", rows.buf.capacity());
        // Exactly at the limit is allowed; one byte over is not.
        let row = format!("[{{\"id\": 1, \"name\": \"{}\"}}]", "x".repeat(1000));
        let size = row.len() - 2;
        let at = JsonRows::new(&report(), Drip(row.as_bytes(), 7)).max_row_bytes(size + 1);
        assert_eq!(at.count(), 1);
        let over = JsonRows::new(&report(), Drip(row.as_bytes(), 7)).max_row_bytes(size - 10);
        assert!(over.collect::<Result<Vec<Record>>>().is_err());
        // Under the limit, the same row is read.
        let rows = JsonRows::new(&report(), Drip(big.as_bytes(), 64 << 10)).max_row_bytes(8 << 20);
        assert_eq!(rows.count(), 1);
    }

    #[test]
    fn streams_arrays_and_lines_the_same_as_whole_documents() {
        let array = r#" [ {"id": 1, "name": "Añasco, \"x\""} ,{"id":2,"name":"b"},
            {"id": 3, "name": null} ] "#;
        let whole = show(records_from_json(&report(), &serde_json::from_str(array).unwrap()).unwrap());
        for step in [1, 2, 7, 4096] {
            assert_eq!(read(array, step).unwrap(), whole, "array, {step} bytes at a time");
            let lines = "\u{feff}{\"id\": 1, \"name\": \"Añasco, \\\"x\\\"\"}\n{\"id\":2,\"name\":\"b\"}\n\n{\"id\": 3, \"name\": null}\n";
            assert_eq!(read(lines, step).unwrap(), whole, "lines, {step} bytes at a time");
        }
        assert!(read("[]", 1).unwrap().is_empty());
        assert!(read("", 1).is_err());
    }

    #[test]
    fn reports_broken_data() {
        assert!(read(r#"[{"id": 1}"#, 3).is_err());
        assert!(read(r#"[{"id": 1} {"id": 2}]"#, 3).is_err());
        assert!(read(r#"[1, 2]"#, 3).is_err());
        assert!(read(r#"{"id": 1"#, 3).is_err());
        assert!(read(r#""rows""#, 3).is_err());
    }
}
