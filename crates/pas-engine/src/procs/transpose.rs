//! PROC TRANSPOSE — long-to-wide pivot, powered by DuckDB's PIVOT.
//!
//! Syntax supported:
//!
//! ```sas
//! proc transpose data=in out=out [prefix=p];
//!     by  group_var [group_var2 ...];
//!     id  column_source_var;
//!     var value_var;
//! run;
//! ```
//!
//! `id` and `var` are required and each take a single variable (multiple
//! `id`/`var` columns are a future enhancement).

use crate::datastep::ast::TableRef;

use super::parse::{parse_name_list, parse_options, split_body};
use super::parse_table_ref;

#[derive(Debug)]
pub struct TransposeSpec {
    pub data_in: TableRef,
    pub data_out: TableRef,
    pub by_vars: Vec<String>,
    pub id_var: String,
    pub value_var: String,
    pub prefix: Option<String>,
}

pub fn parse(body: &str) -> Result<TransposeSpec, String> {
    let (header, rest) = split_body(body);
    let opts = parse_options(&header);
    let data_in = match opts.get("data").and_then(|v| v.as_ref()) {
        Some(s) => parse_table_ref(s).ok_or_else(|| "invalid data=".to_string())?,
        None => return Err("PROC TRANSPOSE requires data=<dataset>".into()),
    };
    let data_out = match opts.get("out").and_then(|v| v.as_ref()) {
        Some(s) => parse_table_ref(s).ok_or_else(|| "invalid out=".to_string())?,
        None => return Err("PROC TRANSPOSE requires out=<dataset>".into()),
    };
    let prefix = opts.get("prefix").and_then(|v| v.clone());

    let mut by_vars: Vec<String> = Vec::new();
    let mut id_var: Option<String> = None;
    let mut value_var: Option<String> = None;
    for stmt in &rest {
        let lower = stmt.to_ascii_lowercase();
        if let Some(rest) = strip_keyword(&lower, stmt, "by") {
            by_vars = parse_name_list(rest);
        } else if let Some(rest) = strip_keyword(&lower, stmt, "id") {
            let names = parse_name_list(rest);
            if names.len() != 1 {
                return Err("PROC TRANSPOSE expects a single id variable".into());
            }
            id_var = Some(names.into_iter().next().unwrap());
        } else if let Some(rest) = strip_keyword(&lower, stmt, "var") {
            let names = parse_name_list(rest);
            if names.len() != 1 {
                return Err("PROC TRANSPOSE expects a single var variable".into());
            }
            value_var = Some(names.into_iter().next().unwrap());
        }
    }
    let id_var = id_var.ok_or("PROC TRANSPOSE requires an `id` statement")?;
    let value_var = value_var.ok_or("PROC TRANSPOSE requires a `var` statement")?;
    Ok(TransposeSpec {
        data_in,
        data_out,
        by_vars,
        id_var,
        value_var,
        prefix,
    })
}

fn strip_keyword<'a>(lower: &str, original: &'a str, kw: &str) -> Option<&'a str> {
    if lower == kw {
        return Some("");
    }
    let prefix = format!("{} ", kw);
    if lower.starts_with(&prefix) {
        return Some(&original[prefix.len()..]);
    }
    None
}

/// Build the DuckDB PIVOT statement for this transpose. The caller wraps
/// it in `CREATE OR REPLACE TABLE … AS` or `COPY (...) TO '…'`.
pub fn build_select_sql(from_clause: &str, spec: &TransposeSpec) -> String {
    let group_by = if spec.by_vars.is_empty() {
        String::new()
    } else {
        let cols = spec
            .by_vars
            .iter()
            .map(|v| format!("\"{}\"", v))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" GROUP BY {}", cols)
    };

    // DuckDB syntax: PIVOT <relation> ON <pivot_col> USING <agg> [GROUP BY …]
    format!(
        "PIVOT {} ON \"{}\" USING first(\"{}\"){}",
        from_clause, spec.id_var, spec.value_var, group_by
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_form() {
        let spec = parse("data=in out=out;by g;id item;var value").unwrap();
        assert_eq!(spec.by_vars, vec!["g".to_string()]);
        assert_eq!(spec.id_var, "item");
        assert_eq!(spec.value_var, "value");
    }

    #[test]
    fn requires_id_and_var() {
        assert!(parse("data=in out=out;by g").is_err());
    }
}
