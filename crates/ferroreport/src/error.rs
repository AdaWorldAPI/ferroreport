//! Error type shared by every stage of the pipeline.

use std::fmt;

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while parsing, filling or exporting a report.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The template is not well-formed XML.
    #[error("invalid XML: {0}")]
    Xml(String),
    /// The XML is valid but does not describe a report we understand.
    #[error("invalid report template: {0}")]
    Template(String),
    /// An expression could not be parsed. `line` and `column` say where it is in the
    /// template (1-based; 0 when it is not known).
    #[error("{}cannot parse expression `{expr}`: {message}", position(*line, *column))]
    ExprParse { expr: String, message: String, line: usize, column: usize },
    /// An expression failed while being evaluated.
    #[error("error evaluating expression: {0}")]
    Eval(String),
    /// The template uses a feature this engine does not implement yet.
    #[error("unsupported feature: {0}")]
    Unsupported(String),
    /// The input data could not be read.
    #[error("invalid data: {0}")]
    Data(String),
    /// Font loading or subsetting failed.
    #[error("font error: {0}")]
    Font(String),
    /// The render went over a limit set for it (pages, time).
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// The output could not be written.
    #[error("cannot write the output: {0}")]
    Io(String),
}

impl Error {
    pub(crate) fn eval(msg: impl fmt::Display) -> Self {
        Error::Eval(msg.to_string())
    }
    pub(crate) fn template(msg: impl fmt::Display) -> Self {
        Error::Template(msg.to_string())
    }
}

fn position(line: usize, column: usize) -> String {
    if line == 0 {
        String::new()
    } else {
        format!("line {line}, column {column}: ")
    }
}

/// The line and column (1-based) of byte `offset` in `source`.
pub fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..source.floor_char_boundary_compat(offset)];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
    (line, column)
}

trait FloorBoundary {
    fn floor_char_boundary_compat(&self, i: usize) -> usize;
}

impl FloorBoundary for str {
    fn floor_char_boundary_compat(&self, i: usize) -> usize {
        let mut i = i.min(self.len());
        while !self.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}
