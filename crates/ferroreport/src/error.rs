//! Error type shared by every stage of the pipeline.

use std::fmt;

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while parsing, filling or exporting a report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The template is not well-formed XML.
    #[error("invalid XML: {0}")]
    Xml(String),
    /// The XML is valid but does not describe a report we understand.
    #[error("invalid report template: {0}")]
    Template(String),
    /// An expression could not be parsed.
    #[error("cannot parse expression `{expr}`: {message}")]
    ExprParse { expr: String, message: String },
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
}

impl Error {
    pub(crate) fn eval(msg: impl fmt::Display) -> Self {
        Error::Eval(msg.to_string())
    }
    pub(crate) fn template(msg: impl fmt::Display) -> Self {
        Error::Template(msg.to_string())
    }
}
