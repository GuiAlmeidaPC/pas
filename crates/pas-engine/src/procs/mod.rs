//! Non-SQL PROCs. Each sub-module owns its own body parser and SQL
//! generation. The dispatcher in `Session::run_proc` (lib.rs) routes by
//! lowercased name.

pub mod parse;
pub mod print;
pub mod sort;
pub mod transpose;

use crate::datastep::ast::TableRef;

/// Reference to a dataset parsed from `data=lib.ds` style options.
pub fn parse_table_ref(s: &str) -> Option<TableRef> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some((l, n)) = s.split_once('.') {
        Some(TableRef {
            libref: Some(l.to_ascii_lowercase()),
            name: n.to_ascii_lowercase(),
            in_var: None,
        })
    } else {
        Some(TableRef {
            libref: None,
            name: s.to_ascii_lowercase(),
            in_var: None,
        })
    }
}
