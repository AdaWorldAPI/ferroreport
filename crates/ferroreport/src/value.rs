//! Runtime values with the semantics JRXML expressions expect from Java.

use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use std::cmp::Ordering;
use std::sync::Arc;

/// A value flowing through fields, parameters, variables and expressions.
#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    /// `java.lang.Integer` (also used for `Short` and `Byte`).
    Int(i32),
    /// `java.lang.Long`.
    Long(i64),
    /// `java.lang.Float`.
    Float(f32),
    /// `java.lang.Double`.
    Double(f64),
    /// `java.math.BigDecimal`.
    Decimal(Decimal),
    Str(Arc<str>),
    /// `java.util.Date` and its `java.sql` subclasses, always in the report time zone.
    Date(NaiveDateTime),
    /// `byte[]` / `InputStream`: binary data such as an image.
    Bytes(Arc<[u8]>),
    /// Structured data from the JSON source: a list of rows (`java.util.List`, a data
    /// source for a subreport) or a nested object.
    Json(Arc<serde_json::Value>),
    /// A list made by an expression: a Groovy `[1, 2, 3]` or what `findAll` and `collect` give.
    List(Arc<Vec<Value>>),
    /// `net.sf.jasperreports.types.date.DateRange`, what `DATERANGE()` gives.
    DateRange(Arc<DateRange>),
    /// A Groovy map (`[a: 1, b: 2]`), its entries in the order they were added.
    Map(Arc<Vec<(Value, Value)>>),
}

/// A span of time, as JasperReports' `DATERANGE("MONTH-1")` or `DATERANGE("2026-03-05")`
/// gives it: `$X{}` clauses compare columns with its start or its end.
#[derive(Clone, Debug, PartialEq)]
pub struct DateRange {
    /// The first instant.
    pub start: NaiveDateTime,
    /// The last instant (the millisecond before the next unit, for a relative range).
    pub end: NaiveDateTime,
    /// How it was written, which is what it prints as.
    pub text: String,
}

/// Java classes a field, parameter or variable can be declared with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaType {
    String,
    Integer,
    Long,
    Short,
    Byte,
    Double,
    Float,
    BigDecimal,
    BigInteger,
    Number,
    Boolean,
    Date,
    Timestamp,
    SqlDate,
    Time,
    /// `byte[]`, `java.io.InputStream`, `java.awt.Image`: binary content.
    Bytes,
    /// `java.util.List`, `Collection`, `JRDataSource`: nested rows.
    List,
    /// `net.sf.jasperreports.types.date.DateRange`.
    DateRange,
    Object,
}

impl JavaType {
    /// Maps a (possibly fully qualified) Java class name to a known type.
    pub fn from_class_name(name: &str) -> JavaType {
        let simple = name.rsplit('.').next().unwrap_or(name);
        let qualified_sql = name.starts_with("java.sql.");
        match simple {
            "String" | "CharSequence" => JavaType::String,
            "Integer" | "int" => JavaType::Integer,
            "Long" | "long" => JavaType::Long,
            "Short" | "short" => JavaType::Short,
            "Byte" | "byte" => JavaType::Byte,
            "Double" | "double" => JavaType::Double,
            "Float" | "float" => JavaType::Float,
            "BigDecimal" => JavaType::BigDecimal,
            "BigInteger" => JavaType::BigInteger,
            "Number" => JavaType::Number,
            "Boolean" | "boolean" => JavaType::Boolean,
            "Timestamp" => JavaType::Timestamp,
            "Time" => JavaType::Time,
            "Date" if qualified_sql => JavaType::SqlDate,
            "Date" | "Calendar" | "LocalDateTime" | "LocalDate" => JavaType::Date,
            "byte[]" | "[B" | "InputStream" | "Image" | "BufferedImage" | "Blob" => JavaType::Bytes,
            "List" | "Collection" | "ArrayList" | "Set" | "JRDataSource" | "JRRewindableDataSource" | "JsonDataSource" => {
                JavaType::List
            }
            "DateRange" => JavaType::DateRange,
            _ => JavaType::Object,
        }
    }

    /// The zero value used to initialise sums of this type.
    pub fn zero(self) -> Value {
        match self {
            JavaType::Integer | JavaType::Short | JavaType::Byte => Value::Int(0),
            JavaType::Long | JavaType::BigInteger => Value::Long(0),
            JavaType::Float => Value::Float(0.0),
            JavaType::BigDecimal => Value::Decimal(Decimal::ZERO),
            _ => Value::Double(0.0),
        }
    }

    /// Converts a numeric value to this declared type (no-op for non-numeric types).
    pub fn coerce(self, v: Value) -> Value {
        if v.is_null() {
            return v;
        }
        match self {
            JavaType::Integer | JavaType::Short | JavaType::Byte => v.as_i64().map(|n| Value::Int(n as i32)).unwrap_or(v),
            JavaType::Long | JavaType::BigInteger => v.as_i64().map(Value::Long).unwrap_or(v),
            JavaType::Double => v.as_f64().map(Value::Double).unwrap_or(v),
            JavaType::Float => v.as_f64().map(|f| Value::Float(f as f32)).unwrap_or(v),
            JavaType::BigDecimal => v.as_decimal().map(Value::Decimal).unwrap_or(v),
            _ => v,
        }
    }
}

impl Value {
    pub fn str(s: impl Into<Arc<str>>) -> Value {
        Value::Str(s.into())
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Double(_) | Value::Decimal(_))
    }

    pub fn as_f64(&self) -> Option<f64> {
        use rust_decimal::prelude::ToPrimitive;
        match self {
            Value::Int(n) => Some(*n as f64),
            Value::Long(n) => Some(*n as f64),
            Value::Float(f) => Some(*f as f64),
            Value::Double(f) => Some(*f),
            Value::Decimal(d) => d.to_f64(),
            _ => None,
        }
    }

    /// Integer view with Java narrowing semantics (truncation toward zero).
    pub fn as_i64(&self) -> Option<i64> {
        use rust_decimal::prelude::ToPrimitive;
        match self {
            Value::Int(n) => Some(*n as i64),
            Value::Long(n) => Some(*n),
            Value::Float(f) => Some(*f as i64),
            Value::Double(f) => Some(*f as i64),
            Value::Decimal(d) => d.trunc().to_i64(),
            _ => None,
        }
    }

    pub fn as_decimal(&self) -> Option<Decimal> {
        match self {
            Value::Int(n) => Some(Decimal::from(*n)),
            Value::Long(n) => Some(Decimal::from(*n)),
            Value::Float(f) => Decimal::from_str_exact(&java_float_to_string(*f)).ok(),
            Value::Double(f) => Decimal::from_str_exact(&shortest_plain(*f)).ok(),
            Value::Decimal(d) => Some(*d),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Java-style `equals`, with numeric values compared by magnitude.
    pub fn java_equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Null, _) | (_, Value::Null) => false,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::List(a), Value::List(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.java_equals(y)),
            (Value::DateRange(a), Value::DateRange(b)) => a.start == b.start && a.end == b.end,
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len() && a.iter().all(|(k, v)| b.iter().any(|(k2, v2)| k.java_equals(k2) && v.java_equals(v2)))
            }
            (a, b) if a.is_numeric() && b.is_numeric() => compare_numbers(a, b) == Some(Ordering::Equal),
            _ => false,
        }
    }

    /// Natural ordering for comparable values (`Comparable.compareTo`).
    pub fn compare(&self, other: &Value) -> Option<Ordering> {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
            (Value::Date(a), Value::Date(b)) => Some(a.cmp(b)),
            (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
            (a, b) if a.is_numeric() && b.is_numeric() => compare_numbers(a, b),
            _ => None,
        }
    }

    /// Name of the Java class of this value, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "Boolean",
            Value::Int(_) => "Integer",
            Value::Long(_) => "Long",
            Value::Float(_) => "Float",
            Value::Double(_) => "Double",
            Value::Decimal(_) => "BigDecimal",
            Value::Str(_) => "String",
            Value::Date(_) => "Date",
            Value::Bytes(_) => "byte[]",
            Value::Json(_) | Value::List(_) => "List",
            Value::DateRange(_) => "DateRange",
            Value::Map(_) => "Map",
        }
    }

    /// `String.valueOf(value)` as Java would print it.
    pub fn to_java_string(&self) -> String {
        match self {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Long(n) => n.to_string(),
            Value::Float(f) => java_float_to_string(*f),
            Value::Double(f) => java_double_to_string(*f),
            Value::Decimal(d) => d.to_string(),
            Value::Str(s) => s.to_string(),
            Value::Date(d) => d.format("%a %b %d %H:%M:%S UTC %Y").to_string(),
            Value::Bytes(b) => format!("[B@{:x}", b.len()),
            Value::Json(j) => j.to_string(),
            // As Groovy prints a list.
            Value::List(items) => format!("[{}]", items.iter().map(Value::to_java_string).collect::<Vec<_>>().join(", ")),
            Value::DateRange(r) => r.text.clone(),
            // As a Java map prints.
            Value::Map(entries) => format!(
                "{{{}}}",
                entries
                    .iter()
                    .map(|(k, v)| format!("{}={}", k.to_java_string(), v.to_java_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

fn compare_numbers(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Decimal(_), _) | (_, Value::Decimal(_)) => Some(a.as_decimal()?.cmp(&b.as_decimal()?)),
        (Value::Int(_) | Value::Long(_), Value::Int(_) | Value::Long(_)) => Some(a.as_i64()?.cmp(&b.as_i64()?)),
        _ => a.as_f64()?.partial_cmp(&b.as_f64()?),
    }
}

/// `a.divide(b, new MathContext(precision, HALF_EVEN))` with Java's scale rules: an exact
/// quotient keeps at least the preferred scale `a.scale() - b.scale()`; an inexact one is
/// rounded to `precision` significant digits.
pub fn java_decimal_divide(a: Decimal, b: Decimal, precision: u32) -> Option<Decimal> {
    use rust_decimal::RoundingStrategy;
    if b.is_zero() {
        return None;
    }
    let q = a.checked_div(b)?;
    let preferred = a.scale().saturating_sub(b.scale());
    let exact = q.checked_mul(b) == Some(a);
    let int_digits = {
        let t = q.abs().trunc();
        if t.is_zero() {
            0
        } else {
            t.to_string().len() as i64
        }
    };
    let leading_zeros = if int_digits == 0 && !q.is_zero() {
        let s = q.abs().to_string();
        let frac = s.split_once('.').map(|(_, f)| f).unwrap_or("");
        frac.chars().take_while(|c| *c == '0').count() as i64
    } else {
        0
    };
    let max_dp = (precision as i64 - int_digits + leading_zeros).clamp(0, 28) as u32;
    let mut r = q.round_dp_with_strategy(max_dp, RoundingStrategy::MidpointNearestEven);
    if exact {
        r = r.normalize();
        if r.scale() < preferred {
            r.rescale(preferred);
        }
    }
    Some(r)
}

/// Splits the shortest round-trip representation of `x` into (digits, decimal exponent),
/// so that `x = 0.d1d2d3... * 10^exp`.
fn shortest_digits(x: f64) -> (String, i32) {
    let s = format!("{:e}", x.abs());
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    (digits, exp + 1)
}

/// Shortest representation of `x` without an exponent (used for exact decimal conversion).
fn shortest_plain(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return "0".to_string();
    }
    let (digits, exp) = shortest_digits(x);
    let sign = if x < 0.0 { "-" } else { "" };
    if exp <= 0 {
        format!("{sign}0.{}{}", "0".repeat((-exp) as usize), digits)
    } else if exp as usize >= digits.len() {
        format!("{sign}{}{}", digits, "0".repeat(exp as usize - digits.len()))
    } else {
        format!("{sign}{}.{}", &digits[..exp as usize], &digits[exp as usize..])
    }
}

/// `Double.toString` (JDK 19+ shortest-digits algorithm).
pub fn java_double_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let (digits, exp) = shortest_digits(x);
    let sign = if x < 0.0 { "-" } else { "" };
    let abs = x.abs();
    if (1e-3..1e7).contains(&abs) {
        if exp <= 0 {
            format!("{sign}0.{}{}", "0".repeat((-exp) as usize), digits)
        } else if exp as usize >= digits.len() {
            format!("{sign}{}{}.0", digits, "0".repeat(exp as usize - digits.len()))
        } else {
            format!("{sign}{}.{}", &digits[..exp as usize], &digits[exp as usize..])
        }
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp - 1)
    }
}

/// `Float.toString`.
pub fn java_float_to_string(x: f32) -> String {
    if !x.is_finite() || x == 0.0 {
        return java_double_to_string(x as f64);
    }
    // Shortest digits for the f32, laid out with Double.toString rules.
    let s = format!("{:e}", x.abs());
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse::<i32>().unwrap_or(0) + 1;
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let sign = if x < 0.0 { "-" } else { "" };
    let abs = x.abs();
    if (1e-3..1e7).contains(&abs) {
        if exp <= 0 {
            format!("{sign}0.{}{}", "0".repeat((-exp) as usize), digits)
        } else if exp as usize >= digits.len() {
            format!("{sign}{}{}.0", digits, "0".repeat(exp as usize - digits.len()))
        } else {
            format!("{sign}{}.{}", &digits[..exp as usize], &digits[exp as usize..])
        }
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_to_string_matches_java() {
        assert_eq!(java_double_to_string(1.0), "1.0");
        assert_eq!(java_double_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(java_double_to_string(1234567.5), "1234567.5");
        assert_eq!(java_double_to_string(1e7), "1.0E7");
        assert_eq!(java_double_to_string(12345678.9), "1.23456789E7");
        assert_eq!(java_double_to_string(0.001), "0.001");
        assert_eq!(java_double_to_string(0.0001), "1.0E-4");
        assert_eq!(java_double_to_string(-2.5), "-2.5");
        assert_eq!(java_double_to_string(100.0), "100.0");
    }

    #[test]
    fn decimal_divide_matches_java_decimal64() {
        let d = |s: &str| s.parse::<Decimal>().unwrap();
        // Expected values printed by Java 21.
        assert_eq!(java_decimal_divide(d("28.837"), d("30"), 16).unwrap().to_string(), "0.9612333333333333");
        assert_eq!(java_decimal_divide(d("10.000"), d("4"), 16).unwrap().to_string(), "2.500");
        assert_eq!(java_decimal_divide(d("10"), d("3"), 16).unwrap().to_string(), "3.333333333333333");
        assert_eq!(java_decimal_divide(d("-5.5"), d("2"), 16).unwrap().to_string(), "-2.75");
    }

    #[test]
    fn numeric_equality_crosses_types() {
        assert!(Value::Int(2).java_equals(&Value::Double(2.0)));
        assert!(!Value::str("2").java_equals(&Value::Int(2)));
    }
}
