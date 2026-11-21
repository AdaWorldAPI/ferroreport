//! Tokenizer for JRXML expressions (Java/Groovy syntax subset).

use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Num(NumLit),
    Str(String),
    Ident(String),
    Field(String),
    Param(String),
    Var(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Dot,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NumLit {
    Int(i32),
    Long(i64),
    Double(f64),
    Float(f32),
    Decimal(String),
}

impl NumLit {
    pub fn into_value(self) -> Value {
        match self {
            NumLit::Int(n) => Value::Int(n),
            NumLit::Long(n) => Value::Long(n),
            NumLit::Double(f) => Value::Double(f),
            NumLit::Float(f) => Value::Float(f),
            NumLit::Decimal(s) => s.parse().map(Value::Decimal).unwrap_or(Value::Null),
        }
    }
}

const OPS: &[&str] = &["?:", "==", "!=", "<=", ">=", "&&", "||", "+", "-", "*", "/", "%", "<", ">", "!", "?", ":"];

/// Splits `src` into tokens. `groovy` changes how unsuffixed decimals are typed.
pub fn tokenize(src: &str, groovy: bool) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Comments.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // $F{name}, $P{name}, $V{name}, $P!{name}
        if c == '$' && i + 2 < chars.len() && matches!(chars[i + 1], 'F' | 'P' | 'V' | 'R') {
            let kind = chars[i + 1];
            let mut j = i + 2;
            if chars.get(j) == Some(&'!') {
                j += 1;
            }
            if chars.get(j) == Some(&'{') {
                let start = j + 1;
                let end = (start..chars.len()).find(|&k| chars[k] == '}').ok_or("unterminated reference")?;
                let name: String = chars[start..end].iter().collect();
                out.push(match kind {
                    'F' => Token::Field(name),
                    'P' => Token::Param(name),
                    'V' => Token::Var(name),
                    _ => return Err(format!("resource references $R{{{name}}} are not supported")),
                });
                i = end + 1;
                continue;
            }
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            if c == '0' && matches!(chars.get(i + 1), Some('x') | Some('X')) {
                i += 2;
                while i < chars.len() && chars[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let hex: String = chars[start + 2..i].iter().collect();
                let n = i64::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                let long = matches!(chars.get(i), Some('L') | Some('l'));
                if long {
                    i += 1;
                }
                out.push(Token::Num(if long { NumLit::Long(n) } else { NumLit::Int(n as i32) }));
                continue;
            }
            let mut is_float = false;
            let digits = |i: &mut usize| {
                while *i < chars.len() && (chars[*i].is_ascii_digit() || chars[*i] == '_') {
                    *i += 1;
                }
            };
            digits(&mut i);
            if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                is_float = true;
                i += 1;
                digits(&mut i);
            }
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                is_float = true;
                i += 1;
                if i < chars.len() && matches!(chars[i], '+' | '-') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            let suffix = chars.get(i).copied();
            let lit = match suffix {
                Some('L') | Some('l') => {
                    i += 1;
                    NumLit::Long(text.parse().map_err(|_| format!("bad long literal {text}"))?)
                }
                Some('d') | Some('D') => {
                    i += 1;
                    NumLit::Double(text.parse().map_err(|_| format!("bad double literal {text}"))?)
                }
                Some('f') | Some('F') => {
                    i += 1;
                    NumLit::Float(text.parse().map_err(|_| format!("bad float literal {text}"))?)
                }
                Some('G') | Some('g') if groovy => {
                    i += 1;
                    NumLit::Decimal(text)
                }
                _ if is_float && groovy => NumLit::Decimal(text),
                _ if is_float => NumLit::Double(text.parse().map_err(|_| format!("bad literal {text}"))?),
                _ => match text.parse::<i32>() {
                    Ok(n) => NumLit::Int(n),
                    Err(_) => NumLit::Long(text.parse().map_err(|_| format!("bad integer literal {text}"))?),
                },
            };
            out.push(Token::Num(lit));
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            let mut s = String::new();
            i += 1;
            loop {
                let Some(&ch) = chars.get(i) else { return Err("unterminated string literal".into()) };
                i += 1;
                if ch == quote {
                    break;
                }
                if ch == '\\' {
                    let Some(&esc) = chars.get(i) else { return Err("bad escape".into()) };
                    i += 1;
                    match esc {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        'b' => s.push('\u{8}'),
                        'f' => s.push('\u{c}'),
                        '0' => s.push('\0'),
                        'u' => {
                            let hex: String = chars.get(i..i + 4).ok_or("bad unicode escape")?.iter().collect();
                            let code = u32::from_str_radix(&hex, 16).map_err(|_| "bad unicode escape")?;
                            s.push(char::from_u32(code).ok_or("bad unicode escape")?);
                            i += 4;
                        }
                        other => s.push(other),
                    }
                } else {
                    s.push(ch);
                }
            }
            out.push(Token::Str(s));
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
            continue;
        }
        match c {
            '(' => out.push(Token::LParen),
            ')' => out.push(Token::RParen),
            ',' => out.push(Token::Comma),
            '.' => out.push(Token::Dot),
            _ => {
                let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
                let op = OPS.iter().find(|op| rest.starts_with(**op)).ok_or_else(|| format!("unexpected character `{c}`"))?;
                out.push(Token::Op(op));
                i += op.chars().count();
                continue;
            }
        }
        i += 1;
    }
    out.push(Token::Eof);
    Ok(out)
}
