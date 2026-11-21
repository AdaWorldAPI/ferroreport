//! `java.text.MessageFormat`, as the `msg()` function of report expressions uses it, and
//! `.properties` files, where the texts of `$R{key}` and `str("key")` come from.

use super::date::format_date;
use super::locale::Locale;
use super::number::NumberPattern;
use crate::value::Value;
use std::collections::HashMap;

/// Formats `pattern` with `args`: `{0}`, `{1,number,#,##0.00}`, `{0,date,dd/MM/yyyy}`...
/// A single quote starts and ends literal text; two of them are one quote.
pub fn message_format(pattern: &str, args: &[Value], locale: &Locale) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut quoted = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
            } else {
                quoted = !quoted;
                i += 1;
            }
        } else if c == '{' && !quoted {
            // To the matching brace: a format of its own may hold braces in quotes.
            let mut depth = 1;
            let mut j = i + 1;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            let inner: String = chars[i + 1..j.saturating_sub(1).max(i + 1)].iter().collect();
            out.push_str(&argument(&inner, args, locale));
            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// One `{index[,type[,style]]}` of a message.
fn argument(inner: &str, args: &[Value], locale: &Locale) -> String {
    let mut parts = inner.splitn(3, ',');
    let index = parts.next().unwrap_or("").trim();
    let kind = parts.next().map(str::trim).unwrap_or("");
    let style = parts.next().map(str::trim).unwrap_or("");
    let Some(value) = index.parse::<usize>().ok().and_then(|n| args.get(n)) else {
        return format!("{{{inner}}}");
    };
    if matches!(value, Value::Null) {
        return "null".into();
    }
    let number = |pattern: &str| NumberPattern::parse(pattern, locale).format(value, locale);
    let date = |pattern: &str| match value {
        Value::Date(d) => format_date(pattern, d, locale),
        _ => None,
    };
    let formatted = match (kind, style) {
        ("number", "" | "number") | ("", _) if value.is_numeric() => number("#,##0.###"),
        ("number", "integer") => number("#,##0"),
        ("number", "percent") => number("#,##0%"),
        ("number", "currency") => number("¤#,##0.00"),
        ("number", pattern) => number(pattern),
        ("date", "" | "medium") => date("MMM d, yyyy"),
        ("date", "short") => date("M/d/yy"),
        ("date", "long") => date("MMMM d, yyyy"),
        ("date", "full") => date("EEEE, MMMM d, yyyy"),
        ("date", pattern) => date(pattern),
        ("time", "" | "medium") => date("h:mm:ss a"),
        ("time", "short") => date("h:mm a"),
        ("time", pattern) => date(pattern),
        ("", _) if matches!(value, Value::Date(_)) => date("M/d/yy, h:mm a"),
        _ => None,
    };
    formatted.unwrap_or_else(|| value.to_java_string())
}

/// The keys and texts of a `.properties` file. The file is ISO-8859-1 with `\uXXXX` for
/// everything else, as Java reads it; a file that is valid UTF-8 is taken as UTF-8, which is
/// what people write today.
pub fn parse_properties(bytes: &[u8]) -> HashMap<String, String> {
    let text: String = match std::str::from_utf8(bytes) {
        Ok(t) => t.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    let mut out = HashMap::new();
    let mut lines = text.lines();
    while let Some(first) = lines.next() {
        let mut line = first.trim_start().to_string();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        // A line that ends in an odd number of backslashes goes on in the next one.
        while line.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1 {
            line.pop();
            match lines.next() {
                Some(next) => line.push_str(next.trim_start()),
                None => break,
            }
        }
        let chars: Vec<char> = line.chars().collect();
        let mut key = String::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '\\' if i + 1 < chars.len() => {
                    i += 1;
                    key.push(chars[i]);
                }
                '=' | ':' | ' ' | '\t' => break,
                c => key.push(c),
            }
            i += 1;
        }
        while i < chars.len() && matches!(chars[i], ' ' | '\t') {
            i += 1;
        }
        if i < chars.len() && matches!(chars[i], '=' | ':') {
            i += 1;
        }
        while i < chars.len() && matches!(chars[i], ' ' | '\t') {
            i += 1;
        }
        let mut value = String::new();
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                i += 1;
                match chars[i] {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    'u' if i + 4 < chars.len() => {
                        let hex: String = chars[i + 1..i + 5].iter().collect();
                        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            Some(c) => value.push(c),
                            None => value.push_str(&format!("\\u{hex}")),
                        }
                        i += 4;
                    }
                    c => value.push(c),
                }
            } else {
                value.push(chars[i]);
            }
            i += 1;
        }
        out.insert(key, value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::locale::EN_US;

    #[test]
    fn messages_are_formatted_like_java() {
        let args = [Value::Double(12.3456), Value::str("Ana"), Value::Int(1234567)];
        assert_eq!(message_format("{0,number,0.00}%", &args, &EN_US), "12.35%");
        assert_eq!(message_format("Hello {1}, you have {2} points", &args, &EN_US), "Hello Ana, you have 1,234,567 points");
        assert_eq!(message_format("It''s {1}; '{0}' stays", &args, &EN_US), "It's Ana; {0} stays");
        assert_eq!(message_format("{2,number,integer} of {5}", &args, &EN_US), "1,234,567 of {5}");
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 9).unwrap().and_hms_opt(14, 5, 0).unwrap();
        assert_eq!(message_format("{0,date,dd/MM/yyyy} {0,time,HH:mm}", &[Value::Date(day)], &EN_US), "09/03/2026 14:05");
    }

    #[test]
    fn properties_files_are_read_like_java() {
        let p = parse_properties(b"# a comment\ntitle = Sales report\npage:Page {0}\nlong = one \\\n   two\nes = Espa\\u00f1a\nempty\nkey\\ with\\ spaces=x\n");
        assert_eq!(p["title"], "Sales report");
        assert_eq!(p["page"], "Page {0}");
        assert_eq!(p["long"], "one two");
        assert_eq!(p["es"], "España");
        assert_eq!(p["empty"], "");
        assert_eq!(p["key with spaces"], "x");
        assert_eq!(parse_properties("año = España".as_bytes())["año"], "España");
        assert_eq!(parse_properties(&[b'a', b'=', 0xF1])["a"], "ñ");
    }
}
