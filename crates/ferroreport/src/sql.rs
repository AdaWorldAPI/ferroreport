//! Rows from a database: the template's SQL query, prepared the way JasperReports prepares it,
//! run on a [`Connection`] the caller provides.
//!
//! In the query text, `$P{name}` becomes a bound parameter (never pasted into the SQL),
//! `$P!{name}` pastes the parameter's text as it is (for an `ORDER BY` column chosen by the
//! caller, say), and `$X{...}` builds a clause from a parameter: `$X{IN, column, param}` and
//! `NOTIN` for a list, and `EQUAL`, `NOTEQUAL`, `LESS`, `LESS]`, `GREATER`, `[GREATER`,
//! `BETWEEN`, `[BETWEEN`, `BETWEEN]` and `[BETWEEN]` for single values. A clause whose
//! parameter is null (or an empty list) is always true, as in JasperReports.
//!
//! The engine itself has no database driver: the command line and the server bring
//! SQLite, PostgreSQL, MySQL and SQL Server, and the Python package any DB-API connection.

use std::collections::HashMap;

use chrono::NaiveDateTime;

use crate::fill::RowIter;
use crate::model::{Field, Report};
use crate::value::Value;
use crate::{Error, Result};

/// A value bound to a query parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A decimal number, as text so no digit is lost.
    Decimal(String),
    Text(String),
    DateTime(NaiveDateTime),
    Bytes(Vec<u8>),
}

/// One row of a result: each column's label and value, in the order of the result.
pub type SqlRow = Vec<(String, serde_json::Value)>;

/// The rows of a result, read as they are needed.
pub type SqlRows<'a> = Box<dyn Iterator<Item = Result<SqlRow>> + 'a>;

/// A database the engine can run queries on.
///
/// Column values are JSON: numbers as numbers (or text, for decimals that must keep every
/// digit), dates and times as ISO 8601 text (`2026-03-05` or `2026-03-05T14:30:00`), binary
/// data (pictures) as base64 text, null as null. Fields then get the class the template
/// declares, as with rows of JSON.
pub trait Connection: Send + Sync {
    /// Runs `sql`, whose `n`-th parameter is written as [`Connection::placeholder`]`(n)`.
    fn query<'a>(&'a self, sql: &str, params: &[SqlValue]) -> Result<SqlRows<'a>>;

    /// How the `n`-th parameter (counted from 1) is written in this database's SQL: `?` for
    /// SQLite and MySQL, `$1` for PostgreSQL, `@P1` for SQL Server.
    fn placeholder(&self, _n: usize) -> String {
        "?".into()
    }
}

/// A connection shared by everything filled in one render.
#[derive(Clone)]
pub struct Shared(pub std::sync::Arc<dyn Connection>);

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Connection")
    }
}

/// A query ready to run: its text and the values of its parameters, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub sql: String,
    pub params: Vec<SqlValue>,
}

/// The value of a parameter as a query parameter.
pub fn sql_value(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Bool(*b),
        Value::Int(i) => SqlValue::Int((*i).into()),
        Value::Long(i) => SqlValue::Int(*i),
        Value::Float(f) => SqlValue::Float((*f).into()),
        Value::Double(f) => SqlValue::Float(*f),
        Value::Decimal(d) => SqlValue::Decimal(d.to_string()),
        Value::Str(s) => SqlValue::Text(s.to_string()),
        Value::Date(d) => SqlValue::DateTime(*d),
        Value::Bytes(b) => SqlValue::Bytes(b.to_vec()),
        Value::Json(j) => match j.as_ref() {
            serde_json::Value::String(s) => SqlValue::Text(s.clone()),
            serde_json::Value::Bool(b) => SqlValue::Bool(*b),
            serde_json::Value::Number(n) => n.as_i64().map(SqlValue::Int).unwrap_or(SqlValue::Float(n.as_f64().unwrap_or(0.0))),
            serde_json::Value::Null => SqlValue::Null,
            other => SqlValue::Text(other.to_string()),
        },
    }
}

/// The items of a list parameter (`$X{IN, ...}`).
fn items(v: &Value) -> Vec<SqlValue> {
    match v {
        Value::Null => Vec::new(),
        Value::Json(j) => match j.as_ref() {
            serde_json::Value::Array(list) => {
                list.iter().map(|x| sql_value(&Value::Json(std::sync::Arc::new(x.clone())))).collect()
            }
            serde_json::Value::Null => Vec::new(),
            _ => vec![sql_value(v)],
        },
        other => vec![sql_value(other)],
    }
}

/// Replaces `$P{}`, `$P!{}` and `$X{}` in a query with placeholders and literal text.
pub fn prepare(text: &str, params: &HashMap<String, Value>, placeholder: &dyn Fn(usize) -> String) -> Result<Prepared> {
    let mut sql = String::with_capacity(text.len());
    let mut bound = Vec::new();
    let param = |name: &str| -> Result<&Value> {
        params.get(name.trim()).ok_or_else(|| {
            Error::Data(format!("the query uses the parameter `{}`, which the template does not declare", name.trim()))
        })
    };
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        sql.push_str(&rest[..start]);
        let after = &rest[start..];
        let (kind, open) = if after.starts_with("$P!{") {
            ("P!", 4)
        } else if after.starts_with("$P{") {
            ("P", 3)
        } else if after.starts_with("$X{") {
            ("X", 3)
        } else {
            sql.push('$');
            rest = &after[1..];
            continue;
        };
        let Some(end) = after[open..].find('}') else {
            return Err(Error::Data(format!("unclosed ${kind}{{ in the query")));
        };
        let inside = &after[open..open + end];
        rest = &after[open + end + 1..];
        match kind {
            "P!" => sql.push_str(&param(inside)?.to_java_string()),
            "P" => {
                bound.push(sql_value(param(inside)?));
                sql.push_str(&placeholder(bound.len()));
            }
            _ => clause(inside, params, placeholder, &mut sql, &mut bound)?,
        }
    }
    sql.push_str(rest);
    Ok(Prepared { sql, params: bound })
}

/// One `$X{function, column, parameter...}` clause.
fn clause(
    inside: &str,
    params: &HashMap<String, Value>,
    placeholder: &dyn Fn(usize) -> String,
    sql: &mut String,
    bound: &mut Vec<SqlValue>,
) -> Result<()> {
    let parts: Vec<&str> = inside.split(',').map(str::trim).collect();
    let (function, column) = match parts.as_slice() {
        [f, c, ..] if parts.len() >= 3 => (f.to_ascii_uppercase(), *c),
        _ => return Err(Error::Data(format!("$X{{{inside}}}: expected $X{{function, column, parameter}}"))),
    };
    let value = |i: usize| -> Result<Value> {
        let name = parts.get(i).copied().unwrap_or_default();
        params.get(name).cloned().ok_or_else(|| Error::Data(format!("$X{{{inside}}}: no parameter `{name}`")))
    };
    let mut bind = |sql: &mut String, v: SqlValue| {
        bound.push(v);
        sql.push_str(&placeholder(bound.len()));
    };
    match function.as_str() {
        "IN" | "NOTIN" => {
            let list = items(&value(2)?);
            if list.is_empty() {
                sql.push_str("0 = 0");
                return Ok(());
            }
            let (nulls, values): (Vec<SqlValue>, Vec<SqlValue>) = list.into_iter().partition(|v| *v == SqlValue::Null);
            let not = function == "NOTIN";
            sql.push('(');
            if !values.is_empty() {
                sql.push_str(column);
                sql.push_str(if not { " NOT IN (" } else { " IN (" });
                for (i, v) in values.into_iter().enumerate() {
                    if i > 0 {
                        sql.push_str(", ");
                    }
                    bind(sql, v);
                }
                sql.push(')');
            }
            if !nulls.is_empty() {
                if sql.ends_with(')') && !sql.ends_with("((") {
                    sql.push_str(if not { " AND " } else { " OR " });
                }
                sql.push_str(column);
                sql.push_str(if not { " IS NOT NULL" } else { " IS NULL" });
            }
            sql.push(')');
        }
        "EQUAL" | "NOTEQUAL" => {
            let v = sql_value(&value(2)?);
            let not = function == "NOTEQUAL";
            sql.push_str(column);
            if v == SqlValue::Null {
                sql.push_str(if not { " IS NOT NULL" } else { " IS NULL" });
            } else {
                sql.push_str(if not { " <> " } else { " = " });
                bind(sql, v);
            }
        }
        "LESS" | "LESS]" | "GREATER" | "[GREATER" => {
            let v = sql_value(&value(2)?);
            if v == SqlValue::Null {
                sql.push_str("0 = 0");
            } else {
                let op = match function.as_str() {
                    "LESS" => " < ",
                    "LESS]" => " <= ",
                    "GREATER" => " > ",
                    _ => " >= ",
                };
                sql.push_str(column);
                sql.push_str(op);
                bind(sql, v);
            }
        }
        "BETWEEN" | "[BETWEEN" | "BETWEEN]" | "[BETWEEN]" => {
            let (low, high) = (sql_value(&value(2)?), sql_value(&value(3)?));
            let lower = if function.starts_with('[') { " >= " } else { " > " };
            let upper = if function.ends_with(']') { " <= " } else { " < " };
            match (low == SqlValue::Null, high == SqlValue::Null) {
                (true, true) => sql.push_str("0 = 0"),
                (false, true) => {
                    sql.push_str(column);
                    sql.push_str(lower);
                    bind(sql, low);
                }
                (true, false) => {
                    sql.push_str(column);
                    sql.push_str(upper);
                    bind(sql, high);
                }
                (false, false) => {
                    sql.push('(');
                    sql.push_str(column);
                    sql.push_str(lower);
                    bind(sql, low);
                    sql.push_str(" AND ");
                    sql.push_str(column);
                    sql.push_str(upper);
                    bind(sql, high);
                    sql.push(')');
                }
            }
        }
        other => return Err(Error::Unsupported(format!("$X{{{other}, ...}} in a query"))),
    }
    Ok(())
}

/// The value of each field in a row, matching fields to columns by label, regardless of
/// case (as JDBC does).
fn row_object(fields: &[Field], row: SqlRow) -> serde_json::Value {
    let mut obj = serde_json::Map::with_capacity(fields.len());
    let mut row: Vec<(String, serde_json::Value)> = row;
    for f in fields {
        if let Some(i) =
            row.iter().position(|(c, _)| c == &f.name).or_else(|| row.iter().position(|(c, _)| c.eq_ignore_ascii_case(&f.name)))
        {
            let (_, v) = row.swap_remove(i);
            obj.insert(f.name.clone(), v);
        }
    }
    serde_json::Value::Object(obj)
}

/// The rows of `report`'s query, run on `connection` with `params` (the report's parameters,
/// defaults included), as records typed by the report's fields.
pub fn query_rows<'a>(report: &Report, params: &HashMap<String, Value>, connection: &'a dyn Connection) -> Result<RowIter<'a>> {
    let query = report.query.as_ref().ok_or_else(|| Error::Data(format!("`{}` has no query", report.name)))?;
    if !query.language.is_empty() && !query.language.eq_ignore_ascii_case("sql") {
        return Err(Error::Unsupported(format!("queries in {}", query.language)));
    }
    let prepared = prepare(&query.text, params, &|n| connection.placeholder(n))?;
    let rows = connection.query(&prepared.sql, &prepared.params)?;
    let fields = report.fields.clone();
    let keep_row = report.uses_data_source;
    let mut index = 0;
    Ok(Box::new(rows.map(move |row| {
        let row = row?;
        index += 1;
        crate::data::record_from_json(&fields, keep_row, index - 1, &row_object(&fields, row))
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn prepares_queries_like_jasperreports() {
        let params = p(&[
            ("id", Value::Int(7)),
            ("order", Value::str("name DESC")),
            ("cities", Value::Json(std::sync::Arc::new(serde_json::json!(["Santo Domingo", "Santiago"])))),
            ("none", Value::Null),
            ("from", Value::Int(10)),
        ]);
        let q = prepare(
            "SELECT * FROM t WHERE id = $P{id} AND $X{IN, city, cities} AND $X{IN, x, none} AND $X{[BETWEEN], n, from, none} ORDER BY $P!{order}",
            &params,
            &|n| format!("${n}"),
        )
        .unwrap();
        assert_eq!(q.sql, "SELECT * FROM t WHERE id = $1 AND (city IN ($2, $3)) AND 0 = 0 AND n >= $4 ORDER BY name DESC");
        assert_eq!(
            q.params,
            vec![SqlValue::Int(7), SqlValue::Text("Santo Domingo".into()), SqlValue::Text("Santiago".into()), SqlValue::Int(10)]
        );
        let q = prepare("a $X{EQUAL, c, none} b $X{NOTEQUAL, c, id}", &params, &|_| "?".into()).unwrap();
        assert_eq!(q.sql, "a c IS NULL b c <> ?");
        assert!(prepare("$P{missing}", &params, &|_| "?".into()).is_err());
        // Text that only looks like a parameter stays as it is.
        assert_eq!(prepare("SELECT '$5' AS price", &params, &|_| "?".into()).unwrap().sql, "SELECT '$5' AS price");
    }
}
