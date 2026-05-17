//! Library (libname) state + dataset listing.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryKind {
    Memory,
    Duckdb,
    Dir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DirFormat {
    Parquet,
    Csv,
}

impl DirFormat {
    pub fn extension(self) -> &'static str {
        match self {
            DirFormat::Parquet => "parquet",
            DirFormat::Csv => "csv",
        }
    }
    pub fn from_ext(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "parquet" => Some(DirFormat::Parquet),
            "csv" => Some(DirFormat::Csv),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub name: String,
    pub kind: LibraryKind,
    /// Filesystem path for DUCKDB/DIR; empty string for MEMORY.
    pub path: String,
    /// Only set for DIR libraries.
    pub format: Option<DirFormat>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetInfo {
    pub libref: String,
    pub name: String,
    pub rows: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnInfo {
    pub name: String,
    pub ty: String,
}
