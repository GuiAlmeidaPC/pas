use crate::library::{DirFormat, Library, LibraryKind};
use crate::quote_ident;
use crate::session::Session;
use std::collections::HashMap;

impl Session {
    /// If `sql` is `CREATE [OR REPLACE] TABLE <libref>.<ds> AS <body>` and
    /// `<libref>` is a DIR library, rewrite the whole statement into a
    /// `COPY (<body>) TO '<path>/<ds>.<ext>' (FORMAT <ext>)`. Otherwise
    /// returns the input unchanged.
    pub(crate) fn rewrite_create_for_dir(&self, sql: &str) -> String {
        let trimmed_start = sql.trim_start();
        let leading_ws = &sql[..sql.len() - trimmed_start.len()];
        let lower = trimmed_start.to_ascii_lowercase();

        let prefix_len = if lower.starts_with("create or replace table") {
            "create or replace table".len()
        } else if lower.starts_with("create table") {
            "create table".len()
        } else {
            return sql.to_string();
        };

        let after_prefix = &trimmed_start[prefix_len..];
        let after_prefix_trim = after_prefix.trim_start();

        // Parse `<libref>.<ds>` (identifier . identifier).
        let mut end_lib = 0;
        for (i, c) in after_prefix_trim.char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end_lib = i + c.len_utf8();
            } else {
                break;
            }
        }
        if end_lib == 0 {
            return sql.to_string();
        }
        let libref = &after_prefix_trim[..end_lib];
        let after_lib = &after_prefix_trim[end_lib..];
        if !after_lib.starts_with('.') {
            return sql.to_string();
        }
        let after_dot = &after_lib[1..];
        let mut end_ds = 0;
        for (i, c) in after_dot.char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end_ds = i + c.len_utf8();
            } else {
                break;
            }
        }
        if end_ds == 0 {
            return sql.to_string();
        }
        let dataset = &after_dot[..end_ds];
        let after_ds = after_dot[end_ds..].trim_start();

        // Need an `as` keyword.
        let lower_after = after_ds.to_ascii_lowercase();
        let body = if let Some(rest) = lower_after.strip_prefix("as ") {
            let consumed = after_ds.len() - rest.len();
            after_ds[consumed..].trim()
        } else if let Some(rest) = lower_after.strip_prefix("as\t") {
            let consumed = after_ds.len() - rest.len();
            after_ds[consumed..].trim()
        } else if lower_after.starts_with("as\n") {
            after_ds[3..].trim()
        } else {
            return sql.to_string();
        };

        let lib = {
            let libs = self.libraries.lock().unwrap_or_else(|e| e.into_inner());
            match libs.get(&libref.to_ascii_lowercase()) {
                Some(l) if l.kind == LibraryKind::Dir => l.clone(),
                _ => return sql.to_string(),
            }
        };

        let fmt = lib.format.unwrap_or(DirFormat::Parquet);
        let ext = fmt.extension();
        let fmt_name = match fmt {
            DirFormat::Parquet => "PARQUET",
            DirFormat::Csv => "CSV",
        };
        let path = format!("{}/{}.{}", lib.path.trim_end_matches('/'), dataset, ext);
        format!(
            "{}COPY ({}) TO '{}' (FORMAT {})",
            leading_ws,
            body,
            path.replace('\'', "''"),
            fmt_name
        )
    }

    /// Replace `libref.dataset` tokens with the actual reader expression
    /// for DIR libraries. DUCKDB libraries already resolve via attached
    /// schemas; MEMORY refs use the default schema.
    pub(crate) fn rewrite_librefs(&self, sql: &str) -> String {
        let libs = self.libraries.lock().unwrap_or_else(|e| e.into_inner());
        let all_libs: Vec<Library> = libs.values().cloned().collect();
        drop(libs);
        if all_libs.is_empty() {
            return sql.to_string();
        }
        let mut out = String::with_capacity(sql.len());
        let bytes = sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            // Skip string literals (honoring PAS-style doubled-quote
            // escapes — '' inside a '...' literal is one apostrophe, not
            // a close-then-open).
            if c == b'\'' || c == b'"' {
                let end = crate::scan::skip_string_literal(bytes, i);
                out.push_str(&sql[i..end]);
                i = end;
                continue;
            }
            if c.is_ascii_alphabetic() || c == b'_' {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let ident = &sql[start..i];
                // libref.dataset?
                if i < bytes.len() && bytes[i] == b'.' {
                    let after_dot = i + 1;
                    let mut j = after_dot;
                    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
                    {
                        j += 1;
                    }
                    if j > after_dot {
                        let ds = &sql[after_dot..j];
                        if let Some(lib) =
                            all_libs.iter().find(|l| l.name.eq_ignore_ascii_case(ident))
                        {
                            match lib.kind {
                                LibraryKind::Dir => {
                                    let reader = dir_reader_expr(lib, ds);
                                    out.push_str(&reader);
                                }
                                LibraryKind::Memory => {
                                    out.push_str(&format!("\"main\".\"{}\"", ds));
                                }
                                LibraryKind::Duckdb => {
                                    out.push_str(&format!("\"{}\".\"{}\"", lib.name, ds));
                                }
                            }
                            i = j;
                            continue;
                        }
                    }
                }
                out.push_str(ident);
                continue;
            }
            out.push(c as char);
            i += 1;
        }
        out
    }
}

/// Build a parameterised `WHERE` clause for the dataset viewer.
///
/// Returns `(sql, params)` where `sql` is either empty or begins with
/// `" WHERE …"` and uses `?` placeholders, and `params` is the
/// corresponding bind values in the same order as the placeholders.
///
/// Why parameterised: user-supplied filter text was previously
/// interpolated literally. While DuckDB doesn't expose dangerous LIKE
/// metacharacters, the unescaped `%` and `_` in filter input behave as
/// SQL wildcards — so a user typing `50%` looks for "any prefix" instead
/// of the literal sequence. Binding the needle with an explicit
/// `ESCAPE '\\'` clause and escaping `\`, `%`, `_` in the value yields
/// the substring search the user expects, and removes the last bit of
/// string-interpolated SQL from the read path.
pub(crate) fn build_where_clause(
    filters: Option<&HashMap<String, String>>,
) -> (String, Vec<String>) {
    let Some(map) = filters else {
        return (String::new(), Vec::new());
    };
    let mut entries: Vec<(String, String)> = map
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if entries.is_empty() {
        return (String::new(), Vec::new());
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut parts = Vec::with_capacity(entries.len());
    let mut params = Vec::with_capacity(entries.len());
    for (col, needle) in entries {
        parts.push(format!(
            "CAST({} AS VARCHAR) ILIKE ? ESCAPE '\\'",
            quote_ident(&col)
        ));
        params.push(format!("%{}%", escape_like(&needle)));
    }
    (format!(" WHERE {}", parts.join(" AND ")), params)
}

/// Escape SQL LIKE/ILIKE metacharacters so a user-supplied substring is
/// matched literally. Pairs with `ESCAPE '\\'` in the surrounding clause.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_filters_produce_empty_clause() {
        let (sql, params) = build_where_clause(None);
        assert!(sql.is_empty());
        assert!(params.is_empty());
    }

    #[test]
    fn empty_values_are_skipped() {
        let mut m = HashMap::new();
        m.insert("a".to_string(), "  ".to_string());
        let (sql, params) = build_where_clause(Some(&m));
        assert!(sql.is_empty());
        assert!(params.is_empty());
    }

    #[test]
    fn single_filter_uses_placeholder_and_escapes_percent() {
        let mut m = HashMap::new();
        m.insert("price".to_string(), "50%".to_string());
        let (sql, params) = build_where_clause(Some(&m));
        assert_eq!(sql, " WHERE CAST(\"price\" AS VARCHAR) ILIKE ? ESCAPE '\\'");
        assert_eq!(params, vec!["%50\\%%".to_string()]);
    }

    #[test]
    fn underscore_and_backslash_are_escaped() {
        let mut m = HashMap::new();
        m.insert("name".to_string(), "a_b\\c".to_string());
        let (_, params) = build_where_clause(Some(&m));
        assert_eq!(params, vec!["%a\\_b\\\\c%".to_string()]);
    }

    #[test]
    fn multiple_filters_are_sorted_by_column_for_determinism() {
        let mut m = HashMap::new();
        m.insert("b".to_string(), "y".to_string());
        m.insert("a".to_string(), "x".to_string());
        let (sql, params) = build_where_clause(Some(&m));
        assert!(sql.starts_with(" WHERE CAST(\"a\" AS VARCHAR)"));
        assert!(sql.contains(" AND CAST(\"b\" AS VARCHAR)"));
        assert_eq!(params, vec!["%x%".to_string(), "%y%".to_string()]);
    }
}

pub(crate) fn dir_reader_expr(lib: &Library, dataset: &str) -> String {
    let fmt = lib.format.unwrap_or(DirFormat::Parquet);
    let path = format!(
        "{}/{}.{}",
        lib.path.trim_end_matches('/'),
        dataset,
        fmt.extension()
    );
    let escaped = path.replace('\'', "''");
    match fmt {
        DirFormat::Parquet => format!("read_parquet('{}')", escaped),
        DirFormat::Csv => format!("read_csv_auto('{}')", escaped),
    }
}

pub(crate) fn dataset_from_clause(
    lib: &Library,
    dataset: &str,
) -> Result<String, crate::EngineError> {
    Ok(match lib.kind {
        LibraryKind::Memory => format!("\"main\".\"{}\"", dataset),
        LibraryKind::Duckdb => format!("\"{}\".\"{}\"", lib.name, dataset),
        LibraryKind::Dir => dir_reader_expr(lib, dataset),
    })
}

pub(crate) fn is_query(sql: &str) -> bool {
    let s = sql.trim_start();
    let head: String = s
        .chars()
        .take_while(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect();
    matches!(
        head.as_str(),
        "select"
            | "with"
            | "show"
            | "describe"
            | "explain"
            | "pragma"
            | "values"
            | "table"
            | "from"
            | "summarize"
    )
}
