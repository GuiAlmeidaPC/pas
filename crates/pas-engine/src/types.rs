use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Source {
        text: String,
    },
    Note {
        text: String,
    },
    Warning {
        text: String,
    },
    Error {
        text: String,
        source_span: Option<SourceSpan>,
    },
    Output {
        block: ResultBlock,
    },
    Done,
}

/// 1-based line/column range in the submitted program (after macro
/// expansion). `start` and `end` mark the offending token.
#[derive(Debug, Clone, Serialize)]
pub struct SourceSpan {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultBlock {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetPage {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    pub total_rows: u64,
}
