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
    #[error("runtime: {0}")]
    Runtime(String),
    #[error("duckdb: {0}")]
    DuckDb(#[from] duckdb::Error),
}
