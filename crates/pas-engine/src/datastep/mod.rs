//! SAS DATA step compiler + interpreter.
//!
//! v0.3 scope (first pass): one input via `set`, assignment, if/then/else,
//! subsetting if, where, keep/drop, length declaration, output/delete, plus
//! a small built-in function library.

pub mod ast;
pub mod exec;
pub mod funcs;
pub mod lex;
pub mod parse;

pub use ast::*;
pub use exec::{run_data_step, DataStepResult};

#[derive(Debug, thiserror::Error)]
pub enum DataStepError {
    #[error("parse: {0}")]
    Parse(String),
    /// Runtime error with an optional span pointing at the offending
    /// expression in the data step body. Span is in body byte offsets;
    /// the caller (Session) maps to absolute source position.
    #[error("runtime: {0}")]
    Runtime(String, Option<lex::Span>),
    #[error("duckdb: {0}")]
    DuckDb(#[from] duckdb::Error),
}

impl DataStepError {
    pub fn runtime(msg: impl Into<String>) -> Self {
        Self::Runtime(msg.into(), None)
    }
    pub fn runtime_at(msg: impl Into<String>, span: lex::Span) -> Self {
        Self::Runtime(msg.into(), Some(span))
    }
}
