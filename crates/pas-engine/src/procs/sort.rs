//! PROC SORT — order a dataset by one or more columns.

use crate::datastep::ast::TableRef;

use super::parse::{parse_by_clause, parse_options, split_body, ByVar};
use super::parse_table_ref;

#[derive(Debug)]
pub struct SortSpec {
    pub data_in: TableRef,
    pub data_out: TableRef,
    pub by_vars: Vec<ByVar>,
    /// `nodupkey` — keep only the first row per by-group.
    pub nodupkey: bool,
    /// `nodupkey noduprecs` / `noduprecs` — remove duplicate full rows.
    pub noduprecs: bool,
}

pub fn parse(body: &str) -> Result<SortSpec, String> {
    let (header, rest) = split_body(body);
    let opts = parse_options(&header);
    let data_in = match opts.get("data").and_then(|v| v.as_ref()) {
        Some(s) => parse_table_ref(s)
            .ok_or_else(|| format!("PROC SORT: invalid data= value {:?}", s))?,
        None => return Err("PROC SORT requires data=<dataset>".into()),
    };
    let data_out = match opts.get("out").and_then(|v| v.as_ref()) {
        Some(s) => parse_table_ref(s)
            .ok_or_else(|| format!("PROC SORT: invalid out= value {:?}", s))?,
        None => data_in.clone(),
    };

    let mut by_vars = Vec::new();
    for stmt in &rest {
        let lower = stmt.to_ascii_lowercase();
        if lower == "by" || lower.starts_with("by ") || lower.starts_with("by\t") {
            let after = &stmt[2..];
            by_vars = parse_by_clause(after);
        } else {
            return Err(format!("PROC SORT: unsupported statement {:?}", stmt));
        }
    }
    if by_vars.is_empty() {
        return Err("PROC SORT requires a `by` statement".into());
    }
    Ok(SortSpec {
        data_in,
        data_out,
        by_vars,
        nodupkey: opts.contains_key("nodupkey"),
        noduprecs: opts.contains_key("noduprecs") || opts.contains_key("nodup"),
    })
}

/// Translate the parsed spec into a `SELECT ... ORDER BY ...` expression
/// suitable for materializing into the output target. The caller wraps
/// this in `CREATE OR REPLACE TABLE ... AS` or `COPY (...) TO 'path'`.
pub fn build_select_sql(from_clause: &str, spec: &SortSpec) -> String {
    let order_by = spec
        .by_vars
        .iter()
        .map(|b| {
            if b.descending {
                format!("\"{}\" DESC", b.name)
            } else {
                format!("\"{}\"", b.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    if spec.nodupkey {
        // Keep the first row per by-key. Use ROW_NUMBER inside the order.
        let partition = spec
            .by_vars
            .iter()
            .map(|b| format!("\"{}\"", b.name))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "SELECT * EXCLUDE (__pas_rn) FROM (\
                SELECT *, ROW_NUMBER() OVER (PARTITION BY {} ORDER BY {}) AS __pas_rn \
                FROM {}\
            ) WHERE __pas_rn = 1 ORDER BY {}",
            partition, order_by, from_clause, order_by
        )
    } else if spec.noduprecs {
        format!(
            "SELECT DISTINCT * FROM {} ORDER BY {}",
            from_clause, order_by
        )
    } else {
        format!("SELECT * FROM {} ORDER BY {}", from_clause, order_by)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_sort() {
        let spec = parse("data=in out=out;by x descending y").unwrap();
        assert_eq!(spec.data_in.name, "in");
        assert_eq!(spec.data_out.name, "out");
        assert_eq!(spec.by_vars.len(), 2);
        assert!(spec.by_vars[1].descending);
    }

    #[test]
    fn sort_in_place_when_out_omitted() {
        let spec = parse("data=people;by name").unwrap();
        assert_eq!(spec.data_out.name, spec.data_in.name);
    }

    #[test]
    fn nodupkey_emits_partition_query() {
        let spec = parse("data=in out=out nodupkey;by id").unwrap();
        let sql = build_select_sql("\"main\".\"in\"", &spec);
        assert!(sql.contains("PARTITION BY"));
        assert!(sql.contains("__pas_rn = 1"));
    }

    #[test]
    fn missing_by_errors() {
        assert!(parse("data=in").is_err());
    }
}
