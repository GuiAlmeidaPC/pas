//! PROC PRINT — emit a dataset to the Output pane.

use crate::datastep::ast::TableRef;

use super::parse::{parse_name_list, parse_options, split_body};
use super::parse_table_ref;

#[derive(Debug)]
pub struct PrintSpec {
    pub data: TableRef,
    pub vars: Vec<String>,
    pub obs: Option<u64>,
}

pub fn parse(body: &str) -> Result<PrintSpec, String> {
    let (header, rest) = split_body(body);
    let opts = parse_options(&header);
    let data = match opts.get("data").and_then(|v| v.as_ref()) {
        Some(s) => parse_table_ref(s)
            .ok_or_else(|| format!("PROC PRINT: invalid data= value {:?}", s))?,
        None => return Err("PROC PRINT requires data=<dataset>".into()),
    };
    let obs = opts
        .get("obs")
        .and_then(|v| v.as_ref())
        .and_then(|s| s.parse::<u64>().ok());

    let mut vars = Vec::new();
    for stmt in &rest {
        let lower = stmt.to_ascii_lowercase();
        if lower.starts_with("var ") || lower == "var" {
            vars = parse_name_list(&stmt[3..]);
        }
        // `where` / `id` etc. silently ignored for v1; not commonly used in
        // wrangling scripts and not part of the spec's PROC PRINT scope.
    }
    Ok(PrintSpec { data, vars, obs })
}

/// Build a SELECT clause that yields the rows PROC PRINT should display.
pub fn build_select_sql(from_clause: &str, spec: &PrintSpec) -> String {
    let cols = if spec.vars.is_empty() {
        "*".to_string()
    } else {
        spec.vars
            .iter()
            .map(|v| format!("\"{}\"", v))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let limit = match spec.obs {
        Some(n) => format!(" LIMIT {}", n),
        None => String::new(),
    };
    format!("SELECT {} FROM {}{}", cols, from_clause, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_var_list_and_obs() {
        let spec = parse("data=in obs=5;var a b c").unwrap();
        assert_eq!(spec.data.name, "in");
        assert_eq!(spec.vars, vec!["a", "b", "c"]);
        assert_eq!(spec.obs, Some(5));
    }

    #[test]
    fn defaults_to_star_when_no_var() {
        let spec = parse("data=foo").unwrap();
        let sql = build_select_sql("\"main\".\"foo\"", &spec);
        assert!(sql.contains("SELECT *"));
    }
}
