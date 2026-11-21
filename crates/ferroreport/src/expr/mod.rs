//! JRXML expression language: a safe, interpreted subset of Java and Groovy.

pub mod eval;
pub mod lexer;
pub mod parser;

pub use eval::{eval, truthy, EvalEnv, Scope};
pub use parser::{parse_expr, Expr};
