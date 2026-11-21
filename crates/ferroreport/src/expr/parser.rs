//! Pratt parser producing an [`Expr`] tree.

use super::lexer::{tokenize, Token};
use crate::value::{JavaType, Value};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
}

/// Parsed expression tree.
#[derive(Debug, Clone)]
pub enum Expr {
    Lit(Value),
    Field(Box<str>),
    Param(Box<str>),
    Var(Box<str>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Elvis(Box<Expr>, Box<Expr>),
    Method { target: Box<Expr>, name: Box<str>, args: Vec<Expr> },
    Static { class: Box<str>, name: Box<str>, args: Vec<Expr> },
    StaticField { class: Box<str>, name: Box<str> },
    New { class: Box<str>, args: Vec<Expr> },
    Cast(JavaType, Box<Expr>),
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// How deep the expression being built is nested (parentheses, operators, calls).
    depth: usize,
}

/// Deepest nesting accepted in an expression. Evaluation recurses once per level, so a
/// template cannot exhaust the stack with `((((...))))` or a chain of thousands of `+`.
const MAX_DEPTH: usize = 256;

const TYPE_NAMES: &[&str] = &[
    "String",
    "Integer",
    "Long",
    "Short",
    "Byte",
    "Double",
    "Float",
    "BigDecimal",
    "BigInteger",
    "Number",
    "Boolean",
    "Date",
    "Timestamp",
    "Object",
    "int",
    "long",
    "double",
    "float",
    "boolean",
    "short",
    "byte",
    "Comparable",
    "List",
    "Collection",
    "Map",
    "JRDataSource",
    "JRRewindableDataSource",
    "JsonDataSource",
    "JRBeanCollectionDataSource",
    "JRMapCollectionDataSource",
];

/// Parses expression source text.
pub fn parse_expr(src: &str, groovy: bool) -> Result<Expr, String> {
    let tokens = tokenize(src, groovy)?;
    let mut p = Parser { tokens, pos: 0, depth: 0 };
    let e = p.expr(0)?;
    match p.peek() {
        Token::Eof => Ok(e),
        t => Err(format!("unexpected token {t:?}")),
    }
}

fn simple_name(qualified: &str) -> &str {
    qualified.rsplit('.').next().unwrap_or(qualified)
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }
    fn peek_at(&self, n: usize) -> &Token {
        self.tokens.get(self.pos + n).unwrap_or(&Token::Eof)
    }
    fn next(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, t: Token) -> Result<(), String> {
        let got = self.next();
        if got == t {
            Ok(())
        } else {
            Err(format!("expected {t:?}, found {got:?}"))
        }
    }

    /// Binding power of infix operators: (left, right).
    fn infix_bp(t: &Token) -> Option<(u8, u8)> {
        Some(match t {
            Token::Op("?") | Token::Op("?:") => (2, 1),
            Token::Op("||") => (3, 4),
            Token::Op("&&") => (5, 6),
            Token::Op("==") | Token::Op("!=") => (7, 8),
            Token::Op("<") | Token::Op("<=") | Token::Op(">") | Token::Op(">=") => (9, 10),
            Token::Op("+") | Token::Op("-") => (11, 12),
            Token::Op("*") | Token::Op("/") | Token::Op("%") => (13, 14),
            _ => return None,
        })
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!("expression nested more than {MAX_DEPTH} levels deep"));
        }
        Ok(())
    }

    fn expr(&mut self, min_bp: u8) -> Result<Expr, String> {
        let start = self.depth;
        let r = self.expr_inner(min_bp);
        self.depth = start;
        r
    }

    fn expr_inner(&mut self, min_bp: u8) -> Result<Expr, String> {
        self.enter()?;
        let mut lhs = self.unary()?;
        loop {
            let op = self.peek().clone();
            let Some((lbp, rbp)) = Self::infix_bp(&op) else { break };
            if lbp < min_bp {
                break;
            }
            // Each operator in a chain nests the expression one level deeper.
            self.enter()?;
            self.next();
            lhs = match op {
                Token::Op("?") => {
                    let then = self.expr(0)?;
                    self.expect(Token::Op(":"))?;
                    let otherwise = self.expr(rbp)?;
                    Expr::Cond(Box::new(lhs), Box::new(then), Box::new(otherwise))
                }
                Token::Op("?:") => Expr::Elvis(Box::new(lhs), Box::new(self.expr(rbp)?)),
                Token::Op("&&") => Expr::And(Box::new(lhs), Box::new(self.expr(rbp)?)),
                Token::Op("||") => Expr::Or(Box::new(lhs), Box::new(self.expr(rbp)?)),
                Token::Op(o) => {
                    let bin = match o {
                        "+" => BinOp::Add,
                        "-" => BinOp::Sub,
                        "*" => BinOp::Mul,
                        "/" => BinOp::Div,
                        "%" => BinOp::Rem,
                        "==" => BinOp::Eq,
                        "!=" => BinOp::Ne,
                        "<" => BinOp::Lt,
                        "<=" => BinOp::Le,
                        ">" => BinOp::Gt,
                        ">=" => BinOp::Ge,
                        _ => return Err(format!("unknown operator {o}")),
                    };
                    Expr::Binary(bin, Box::new(lhs), Box::new(self.expr(rbp)?))
                }
                _ => unreachable!(),
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        let start = self.depth;
        let r = self.unary_inner();
        self.depth = start;
        r
    }

    fn unary_inner(&mut self) -> Result<Expr, String> {
        self.enter()?;
        match self.peek().clone() {
            Token::Op("-") => {
                self.next();
                Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)))
            }
            Token::Op("+") => {
                self.next();
                Ok(Expr::Unary(UnOp::Plus, Box::new(self.unary()?)))
            }
            Token::Op("!") => {
                self.next();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)))
            }
            Token::LParen if self.is_cast() => {
                self.next();
                let name = self.qualified_name()?;
                self.expect(Token::RParen)?;
                let inner = self.unary()?;
                Ok(Expr::Cast(JavaType::from_class_name(&name), Box::new(inner)))
            }
            _ => self.postfix(),
        }
    }

    /// `(Type) operand` — a parenthesised known type name followed by something that can
    /// start an operand.
    fn is_cast(&self) -> bool {
        let mut n = 1;
        let mut last;
        loop {
            match self.peek_at(n) {
                Token::Ident(s) => last = s.clone(),
                _ => return false,
            }
            n += 1;
            match self.peek_at(n) {
                Token::Dot => n += 1,
                Token::RParen => break,
                _ => return false,
            }
        }
        if !TYPE_NAMES.contains(&last.as_str()) {
            return false;
        }
        matches!(
            self.peek_at(n + 1),
            Token::Ident(_)
                | Token::Field(_)
                | Token::Param(_)
                | Token::Var(_)
                | Token::Str(_)
                | Token::Num(_)
                | Token::LParen
                | Token::Op("!")
        )
    }

    fn qualified_name(&mut self) -> Result<String, String> {
        let mut name = match self.next() {
            Token::Ident(s) => s,
            t => return Err(format!("expected identifier, found {t:?}")),
        };
        while self.peek() == &Token::Dot && matches!(self.peek_at(1), Token::Ident(_)) {
            self.next();
            if let Token::Ident(s) = self.next() {
                name.push('.');
                name.push_str(&s);
            }
        }
        Ok(name)
    }

    fn args(&mut self) -> Result<Vec<Expr>, String> {
        self.expect(Token::LParen)?;
        let mut args = Vec::new();
        if self.peek() == &Token::RParen {
            self.next();
            return Ok(args);
        }
        loop {
            args.push(self.expr(0)?);
            match self.next() {
                Token::Comma => continue,
                Token::RParen => break,
                t => return Err(format!("expected `,` or `)`, found {t:?}")),
            }
        }
        Ok(args)
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        while self.peek() == &Token::Dot {
            self.next();
            let name = match self.next() {
                Token::Ident(s) => s,
                t => return Err(format!("expected member name, found {t:?}")),
            };
            let args = if self.peek() == &Token::LParen { self.args()? } else { Vec::new() };
            e = Expr::Method { target: Box::new(e), name: name.into(), args };
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Token::Num(n) => Ok(Expr::Lit(n.into_value())),
            Token::Str(s) => Ok(Expr::Lit(Value::str(s))),
            Token::Field(n) => Ok(Expr::Field(n.into())),
            Token::Param(n) => Ok(Expr::Param(n.into())),
            Token::Var(n) => Ok(Expr::Var(n.into())),
            Token::LParen => {
                let e = self.expr(0)?;
                self.expect(Token::RParen)?;
                Ok(e)
            }
            Token::Ident(id) => match id.as_str() {
                "true" => Ok(Expr::Lit(Value::Bool(true))),
                "false" => Ok(Expr::Lit(Value::Bool(false))),
                "null" => Ok(Expr::Lit(Value::Null)),
                "new" => {
                    let class = self.qualified_name()?;
                    let args = self.args()?;
                    Ok(Expr::New { class: simple_name(&class).into(), args })
                }
                _ => {
                    // Qualified static access: [pkg.]Class.member[(args)]
                    let mut parts = vec![id];
                    while self.peek() == &Token::Dot && matches!(self.peek_at(1), Token::Ident(_)) {
                        // Stop before a lowercase segment that is followed by `(` only when we
                        // already have a class name (handled below by splitting).
                        self.next();
                        if let Token::Ident(s) = self.next() {
                            parts.push(s);
                        }
                        if self.peek() == &Token::LParen {
                            break;
                        }
                    }
                    if parts.len() < 2 {
                        return Err(format!("unknown identifier `{}`", parts[0]));
                    }
                    let member = parts.pop().unwrap_or_default();
                    let class = parts.last().cloned().unwrap_or_default();
                    if self.peek() == &Token::LParen {
                        let args = self.args()?;
                        Ok(Expr::Static { class: class.into(), name: member.into(), args })
                    } else {
                        Ok(Expr::StaticField { class: class.into(), name: member.into() })
                    }
                }
            },
            t => Err(format!("unexpected token {t:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_expressions_nested_too_deep() {
        let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
        assert!(parse_expr(&deep, false).unwrap_err().contains("nested"));
        let chain = vec!["\"a\""; 100_000].join(" + ");
        assert!(parse_expr(&chain, false).is_err());
        assert!(parse_expr(&"-".repeat(100_000), false).is_err());
        let fine = vec!["1"; 100].join(" + ");
        assert!(parse_expr(&fine, false).is_ok());
    }

    #[test]
    fn parses_typical_expressions() {
        for src in [
            "\"Page \" + $V{PAGE_NUMBER} + \" of \"",
            "$F{amount} != null ? $F{amount} : 0",
            "new java.text.SimpleDateFormat(\"dd/MM/yyyy\").format($F{date})",
            "$F{price}.multiply(new java.math.BigDecimal(\"1.18\")).setScale(2, java.math.RoundingMode.HALF_UP)",
            "java.lang.Math.max($F{a}, 10)",
            "((Integer) $V{REPORT_COUNT}).intValue() % 2 == 0",
            "!$F{active}.booleanValue() && $F{name}.trim().length() > 0",
            "$F{x} ?: \"n/a\"",
            "Boolean.TRUE",
        ] {
            parse_expr(src, false).unwrap_or_else(|e| panic!("{src}: {e}"));
        }
    }

    #[test]
    fn precedence() {
        let e = parse_expr("1 + 2 * 3 == 7 && true", false).unwrap();
        assert!(matches!(e, Expr::And(..)));
    }
}
